use std::time::Duration;

/// Result of checking a termination condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationResult {
    /// Continue execution.
    Continue,
    /// Stop execution with a reason.
    Stop(String),
}

/// A composable termination condition.
pub trait TerminationCondition: Send + Sync {
    fn check(&self, iterations: usize, tokens_used: usize, elapsed: Duration) -> TerminationResult;
    fn reset(&mut self) {}
}

/// Stop after max iterations.
pub struct MaxIterations {
    pub max: usize,
}

impl TerminationCondition for MaxIterations {
    fn check(&self, iterations: usize, _tokens_used: usize, _elapsed: Duration) -> TerminationResult {
        if iterations >= self.max {
            TerminationResult::Stop(format!("Max iterations ({}) reached", self.max))
        } else {
            TerminationResult::Continue
        }
    }
}

/// Stop after max tokens.
pub struct MaxTokens {
    pub max: usize,
}

impl TerminationCondition for MaxTokens {
    fn check(&self, _iterations: usize, tokens_used: usize, _elapsed: Duration) -> TerminationResult {
        if tokens_used >= self.max {
            TerminationResult::Stop(format!("Max tokens ({}) reached", self.max))
        } else {
            TerminationResult::Continue
        }
    }
}

/// Stop after timeout.
pub struct Timeout {
    pub duration: Duration,
}

impl TerminationCondition for Timeout {
    fn check(&self, _iterations: usize, _tokens_used: usize, elapsed: Duration) -> TerminationResult {
        if elapsed >= self.duration {
            TerminationResult::Stop(format!("Timeout ({:?}) reached", self.duration))
        } else {
            TerminationResult::Continue
        }
    }
}

/// Combine conditions with AND logic (all must agree to continue).
pub struct AndCondition {
    pub conditions: Vec<Box<dyn TerminationCondition>>,
}

impl TerminationCondition for AndCondition {
    fn check(&self, iterations: usize, tokens_used: usize, elapsed: Duration) -> TerminationResult {
        for condition in &self.conditions {
            match condition.check(iterations, tokens_used, elapsed) {
                TerminationResult::Stop(reason) => return TerminationResult::Stop(reason),
                TerminationResult::Continue => continue,
            }
        }
        TerminationResult::Continue
    }

    fn reset(&mut self) {
        for condition in &mut self.conditions {
            condition.reset();
        }
    }
}

/// Combine conditions with OR logic (any can stop).
pub struct OrCondition {
    pub conditions: Vec<Box<dyn TerminationCondition>>,
}

impl TerminationCondition for OrCondition {
    fn check(&self, iterations: usize, tokens_used: usize, elapsed: Duration) -> TerminationResult {
        for condition in &self.conditions {
            match condition.check(iterations, tokens_used, elapsed) {
                TerminationResult::Stop(reason) => return TerminationResult::Stop(reason),
                TerminationResult::Continue => continue,
            }
        }
        TerminationResult::Continue
    }

    fn reset(&mut self) {
        for condition in &mut self.conditions {
            condition.reset();
        }
    }
}

/// Create default termination conditions for an agent.
pub fn default_termination(max_iterations: usize) -> Box<dyn TerminationCondition> {
    Box::new(AndCondition {
        conditions: vec![
            Box::new(MaxIterations { max: max_iterations }),
            Box::new(Timeout { duration: Duration::from_secs(300) }),
        ],
    })
}
