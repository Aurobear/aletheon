// crates/runtime/src/core/react_loop/reflection.rs
use tracing::info;

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
}

/// Result of a reflection.
#[derive(Debug, Clone)]
pub struct ReflectionResult {
    pub summary: String,
    pub recommendation: ReflectionRecommendation,
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

        let recommendation = if error_rate > 0.5 {
            self.should_stop = true;
            ReflectionRecommendation::Stop(TerminationReason::StuckInLoop)
        } else if context.tool_calls_made >= 10 {
            self.should_stop = true;
            ReflectionRecommendation::Stop(TerminationReason::BudgetExhausted)
        } else {
            ReflectionRecommendation::Continue
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

        let summary = format!(
            "Reflection: {} tool calls made, {} errors ({:.0}% error rate). {}",
            context.tool_calls_made,
            context.errors,
            error_rate * 100.0,
            status_str,
        );

        ReflectionResult {
            summary,
            recommendation,
        }
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

        let ctx = ReflectionContext {
            goal: Some("test".into()),
            recent_actions: vec![],
            current_state: "ok".into(),
            tool_calls_made: 2,
            errors: 0,
        };
        engine.reflect(&ctx);

        assert!(!engine.should_reflect());
        assert_eq!(engine.calls_since_reflection, 0);
    }

    #[test]
    fn test_high_error_rate_stops() {
        let mut engine = ReflectionEngine::new(5);

        let ctx = ReflectionContext {
            goal: Some("test".into()),
            recent_actions: vec![],
            current_state: "error".into(),
            tool_calls_made: 10,
            errors: 6,
        };

        let result = engine.reflect(&ctx);
        assert!(matches!(
            result.recommendation,
            ReflectionRecommendation::Stop(TerminationReason::StuckInLoop)
        ));
    }
}
