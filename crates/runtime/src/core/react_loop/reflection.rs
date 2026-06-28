// crates/runtime/src/core/react_loop/reflection.rs
use tracing::{info, warn};

/// Classifies whether an agent deviation is a bug or an enhancement.
///
/// Inspired by carlos's AI Native article: when agent output diverges from spec,
/// the fix depends on *which side* diverged:
/// - Spec intent is wrong → fix the spec (SpecDeviation)
/// - Agent did extra work → review and absorb or reject (AgentEnhancement)
#[derive(Debug, Clone, PartialEq)]
pub enum SpecVerdict {
    /// Agent output contradicts spec intent — the spec needs fixing.
    SpecDeviation { detail: String },
    /// Agent did extra work beyond spec — review for absorption.
    AgentEnhancement { detail: String },
    /// On track, no deviation detected.
    OnTrack,
}

/// Recommendation from reflection.
#[derive(Debug, Clone)]
pub enum ReflectionRecommendation {
    /// Continue with current strategy.
    Continue,
    /// Adjust strategy with suggestion.
    AdjustStrategy(String),
    /// Stop with reason.
    Stop(TerminationReason),
}

/// Reason for stopping (named differently to avoid collision with cognit::StopReason).
#[derive(Debug, Clone)]
pub enum TerminationReason {
    /// Goal achieved.
    GoalAchieved,
    /// Stuck in a loop.
    StuckInLoop,
    /// Budget exhausted.
    BudgetExhausted,
    /// Timeout.
    Timeout,
    /// User request.
    UserRequest,
}

/// Context provided to the reflection engine.
#[derive(Debug, Clone)]
pub struct ReflectionContext {
    pub goal: Option<String>,
    pub recent_actions: Vec<String>,
    pub current_state: String,
    pub tool_calls_made: usize,
    pub errors: usize,
    /// Constraints from the spec (hard boundaries).
    pub constraints: Vec<String>,
    /// Test failure messages (if any).
    pub test_failures: Vec<String>,
    /// Outputs that go beyond what the spec defined.
    pub unexpected_outputs: Vec<String>,
}

/// Result of a reflection.
#[derive(Debug, Clone)]
pub struct ReflectionResult {
    pub summary: String,
    pub recommendation: ReflectionRecommendation,
    /// Whether the deviation is a spec problem or an agent enhancement.
    pub spec_verdict: SpecVerdict,
}

/// Periodic reflection engine for the agent loop.
#[derive(Debug)]
pub struct ReflectionEngine {
    reflection_interval: usize,
    calls_since_reflection: usize,
    should_stop: bool,
}

impl ReflectionEngine {
    /// Create a new reflection engine.
    /// - reflection_interval: reflect every N tool calls
    pub fn new(reflection_interval: usize) -> Self {
        Self {
            reflection_interval,
            calls_since_reflection: 0,
            should_stop: false,
        }
    }

    /// Check if the last reflection recommended stopping.
    pub fn should_stop(&self) -> bool {
        self.should_stop
    }

    /// Check if it's time to reflect.
    pub fn should_reflect(&self) -> bool {
        self.calls_since_reflection >= self.reflection_interval
    }

    /// Record a tool call and check if reflection is needed.
    pub fn record_call(&mut self) -> bool {
        self.calls_since_reflection += 1;
        self.should_reflect()
    }

    /// Perform reflection and return recommendation.
    pub fn reflect(&mut self, context: &ReflectionContext) -> ReflectionResult {
        info!(
            tool_calls = context.tool_calls_made,
            errors = context.errors,
            "Performing reflection"
        );

        self.calls_since_reflection = 0;

        // Analyze the situation
        let error_rate = if context.tool_calls_made > 0 {
            context.errors as f64 / context.tool_calls_made as f64
        } else {
            0.0
        };

        // Classify deviation: spec problem vs agent enhancement
        let spec_verdict = self.classify_deviation(context);

        let recommendation = if error_rate > 0.5 {
            self.should_stop = true;
            ReflectionRecommendation::Stop(TerminationReason::StuckInLoop)
        } else if context.tool_calls_made >= 10 {
            self.should_stop = true;
            ReflectionRecommendation::Stop(TerminationReason::BudgetExhausted)
        } else {
            // Use spec verdict to guide strategy adjustment
            match &spec_verdict {
                SpecVerdict::SpecDeviation { detail } => {
                    warn!(detail = detail.as_str(), "Spec deviation detected");
                    ReflectionRecommendation::AdjustStrategy(format!(
                        "Spec deviation: {}. Fix the spec, not the code.",
                        detail
                    ))
                }
                SpecVerdict::AgentEnhancement { detail } => {
                    info!(detail = detail.as_str(), "Agent enhancement detected");
                    ReflectionRecommendation::AdjustStrategy(format!(
                        "Agent enhancement: {}. Review: absorb into spec if reasonable, reject if not.",
                        detail
                    ))
                }
                SpecVerdict::OnTrack => ReflectionRecommendation::Continue,
            }
        };

        let status_str = match &recommendation {
            ReflectionRecommendation::Continue => "Continuing...".to_string(),
            ReflectionRecommendation::AdjustStrategy(s) => {
                format!("Adjusting: {}", s)
            }
            ReflectionRecommendation::Stop(reason) => {
                format!("Stopping: {:?}", reason)
            }
        };

        let verdict_str = match &spec_verdict {
            SpecVerdict::SpecDeviation { .. } => "spec deviation",
            SpecVerdict::AgentEnhancement { .. } => "agent enhancement",
            SpecVerdict::OnTrack => "on track",
        };

        let summary = format!(
            "Reflection: {} tool calls made, {} errors ({:.0}% error rate). Spec: {}. {}",
            context.tool_calls_made,
            context.errors,
            error_rate * 100.0,
            verdict_str,
            status_str,
        );

        ReflectionResult {
            summary,
            recommendation,
            spec_verdict,
        }
    }

    /// Classify whether agent output diverged from spec intent.
    ///
    /// Three outcomes:
    /// - SpecDeviation: agent contradicts constraints or test failures indicate
    ///   the spec's expected state is wrong → fix the spec
    /// - AgentEnhancement: agent did more than spec asked → review for absorption
    /// - OnTrack: everything aligns
    fn classify_deviation(&self, ctx: &ReflectionContext) -> SpecVerdict {
        // 1. Check constraint violations — highest priority
        for constraint in &ctx.constraints {
            let constraint_lower = constraint.to_lowercase();
            for action in &ctx.recent_actions {
                if action.to_lowercase().contains(&constraint_lower)
                    || constraint_lower.contains(&action.to_lowercase())
                {
                    return SpecVerdict::SpecDeviation {
                        detail: format!(
                            "Action '{}' may violate constraint '{}'",
                            action, constraint
                        ),
                    };
                }
            }
        }

        // 2. Check test failures against success criteria
        if !ctx.test_failures.is_empty() {
            // If test failures exist, it's a spec deviation —
            // the spec defined an expected state that can't be reached as-is
            return SpecVerdict::SpecDeviation {
                detail: format!(
                    "Test failures suggest spec inconsistency: {}",
                    ctx.test_failures.join("; ")
                ),
            };
        }

        // 3. Check for unexpected enhancements
        if !ctx.unexpected_outputs.is_empty() {
            return SpecVerdict::AgentEnhancement {
                detail: format!(
                    "Agent produced beyond spec: {}",
                    ctx.unexpected_outputs.join("; ")
                ),
            };
        }

        SpecVerdict::OnTrack
    }

    /// Reset for a new turn.
    pub fn reset(&mut self) {
        self.calls_since_reflection = 0;
        self.should_stop = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(tool_calls: usize, errors: usize) -> ReflectionContext {
        ReflectionContext {
            goal: Some("test".into()),
            recent_actions: vec![],
            current_state: "ok".into(),
            tool_calls_made: tool_calls,
            errors,
            constraints: vec![],
            test_failures: vec![],
            unexpected_outputs: vec![],
        }
    }

    #[test]
    fn test_reflection_interval() {
        let mut engine = ReflectionEngine::new(3);

        assert!(!engine.should_reflect());
        engine.record_call();
        assert!(!engine.should_reflect());
        engine.record_call();
        assert!(!engine.should_reflect());
        engine.record_call();
        assert!(engine.should_reflect());
    }

    #[test]
    fn test_reflection_resets_counter() {
        let mut engine = ReflectionEngine::new(2);

        engine.record_call();
        engine.record_call();
        assert!(engine.should_reflect());

        let ctx = make_ctx(2, 0);
        engine.reflect(&ctx);

        assert!(!engine.should_reflect());
        assert_eq!(engine.calls_since_reflection, 0);
    }

    #[test]
    fn test_high_error_rate_stops() {
        let mut engine = ReflectionEngine::new(5);

        let ctx = make_ctx(10, 6);
        let result = engine.reflect(&ctx);
        assert!(matches!(
            result.recommendation,
            ReflectionRecommendation::Stop(TerminationReason::StuckInLoop)
        ));
    }

    #[test]
    fn test_spec_deviation_on_constraint_violation() {
        let mut engine = ReflectionEngine::new(5);

        let ctx = ReflectionContext {
            goal: Some("Deploy service".into()),
            recent_actions: vec!["modify user schema".into()],
            current_state: "ok".into(),
            tool_calls_made: 3,
            errors: 0,
            constraints: vec!["Do not modify user schema".into()],
            test_failures: vec![],
            unexpected_outputs: vec![],
        };

        let result = engine.reflect(&ctx);
        assert!(matches!(result.spec_verdict, SpecVerdict::SpecDeviation { .. }));
        assert!(matches!(
            result.recommendation,
            ReflectionRecommendation::AdjustStrategy(_)
        ));
    }

    #[test]
    fn test_spec_deviation_on_test_failures() {
        let mut engine = ReflectionEngine::new(5);

        let ctx = ReflectionContext {
            goal: Some("Build API".into()),
            recent_actions: vec![],
            current_state: "error".into(),
            tool_calls_made: 5,
            errors: 1,
            constraints: vec![],
            test_failures: vec!["test_login_returns_401".into()],
            unexpected_outputs: vec![],
        };

        let result = engine.reflect(&ctx);
        assert!(matches!(result.spec_verdict, SpecVerdict::SpecDeviation { .. }));
    }

    #[test]
    fn test_agent_enhancement() {
        let mut engine = ReflectionEngine::new(5);

        let ctx = ReflectionContext {
            goal: Some("Build API".into()),
            recent_actions: vec![],
            current_state: "ok".into(),
            tool_calls_made: 4,
            errors: 0,
            constraints: vec![],
            test_failures: vec![],
            unexpected_outputs: vec!["Added rate limiting middleware".into()],
        };

        let result = engine.reflect(&ctx);
        assert!(matches!(result.spec_verdict, SpecVerdict::AgentEnhancement { .. }));
        assert!(matches!(
            result.recommendation,
            ReflectionRecommendation::AdjustStrategy(_)
        ));
    }

    #[test]
    fn test_on_track() {
        let mut engine = ReflectionEngine::new(5);

        let ctx = make_ctx(3, 0);
        let result = engine.reflect(&ctx);
        assert!(matches!(result.spec_verdict, SpecVerdict::OnTrack));
        assert!(matches!(
            result.recommendation,
            ReflectionRecommendation::Continue
        ));
    }
}
