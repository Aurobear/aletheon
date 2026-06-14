//! Reflector — post-execution reflection.
//!
//! Analyzes execution results to determine what worked, what failed,
//! and what should be improved. Produces Reflection structs that feed
//! into the Learner for rule extraction.

use aletheon_abi::brain::{ExecutionResult, Reflection};

/// The reflector component.
///
/// Performs post-execution analysis to extract lessons from outcomes.
pub struct Reflector;

impl Reflector {
    pub fn new() -> Self {
        Self
    }

    /// Reflect on an execution result.
    pub fn reflect(&self, execution: &ExecutionResult) -> Reflection {
        let mut what_worked = Vec::new();
        let mut what_failed = Vec::new();
        let mut what_to_improve = Vec::new();

        // Analyze success/failure
        if execution.success {
            what_worked.push(format!(
                "Plan {} completed successfully ({}/{} steps).",
                execution.plan_id, execution.steps_completed, execution.steps_total
            ));

            if execution.steps_completed < execution.steps_total {
                what_to_improve.push(format!(
                    "Only {}/{} steps completed — investigate partial completion.",
                    execution.steps_completed, execution.steps_total
                ));
            }
        } else {
            what_failed.push(format!(
                "Plan {} failed at step {}/{}.",
                execution.plan_id, execution.steps_completed, execution.steps_total
            ));

            if let Some(ref error) = execution.error {
                what_failed.push(format!("Error: {}", error));
            }

            what_to_improve.push("Add error handling and retry logic for failed steps.".to_string());
            what_to_improve.push("Consider adding rollback actions for partial completion.".to_string());
        }

        // Analyze performance
        if execution.elapsed_ms > 30_000 {
            what_to_improve.push(format!(
                "Execution took {}ms — consider optimizing or parallelizing.",
                execution.elapsed_ms
            ));
        }

        // Analyze output
        if execution.output.is_empty() {
            what_to_improve.push("Execution produced no output — verify expected outcomes.".to_string());
        }

        // Compute confidence
        let confidence = self.compute_confidence(execution);

        Reflection {
            what_worked,
            what_failed,
            what_to_improve,
            confidence,
        }
    }

    /// Compute confidence score (0.0 to 1.0) based on execution outcome.
    fn compute_confidence(&self, execution: &ExecutionResult) -> f64 {
        if execution.steps_total == 0 {
            return 0.0;
        }

        let completion_ratio = execution.steps_completed as f64 / execution.steps_total as f64;
        let success_bonus = if execution.success { 0.2 } else { 0.0 };

        // Penalize long execution times
        let time_penalty = if execution.elapsed_ms > 60_000 {
            0.1
        } else if execution.elapsed_ms > 30_000 {
            0.05
        } else {
            0.0
        };

        (completion_ratio * 0.8 + success_bonus - time_penalty).clamp(0.0, 1.0)
    }
}

impl Default for Reflector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_execution(success: bool, completed: usize, total: usize) -> ExecutionResult {
        ExecutionResult {
            plan_id: Uuid::new_v4(),
            success,
            steps_completed: completed,
            steps_total: total,
            output: "some output".to_string(),
            error: if success { None } else { Some("test error".to_string()) },
            elapsed_ms: 100,
        }
    }

    #[test]
    fn successful_full_reflection() {
        let reflector = Reflector::new();
        let result = reflector.reflect(&make_execution(true, 3, 3));
        assert!(!result.what_worked.is_empty());
        assert!(result.what_failed.is_empty());
        assert!(result.confidence > 0.8);
    }

    #[test]
    fn failed_reflection() {
        let reflector = Reflector::new();
        let result = reflector.reflect(&make_execution(false, 1, 3));
        assert!(!result.what_failed.is_empty());
        assert!(result.what_failed.iter().any(|f| f.contains("Error")));
        assert!(!result.what_to_improve.is_empty());
        assert!(result.confidence < 0.5);
    }

    #[test]
    fn partial_completion_suggests_improvement() {
        let reflector = Reflector::new();
        let result = reflector.reflect(&make_execution(true, 2, 5));
        assert!(result.what_to_improve.iter().any(|i| i.contains("partial")));
    }

    #[test]
    fn slow_execution_suggests_optimization() {
        let reflector = Reflector::new();
        let mut exec = make_execution(true, 1, 1);
        exec.elapsed_ms = 45_000;
        let result = reflector.reflect(&exec);
        assert!(result.what_to_improve.iter().any(|i| i.contains("optimizing")));
    }

    #[test]
    fn empty_output_noted() {
        let reflector = Reflector::new();
        let mut exec = make_execution(true, 1, 1);
        exec.output = String::new();
        let result = reflector.reflect(&exec);
        assert!(result.what_to_improve.iter().any(|i| i.contains("no output")));
    }

    #[test]
    fn zero_steps_zero_confidence() {
        let reflector = Reflector::new();
        let exec = make_execution(true, 0, 0);
        let result = reflector.reflect(&exec);
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn confidence_range() {
        let reflector = Reflector::new();
        // Full success
        let r1 = reflector.reflect(&make_execution(true, 5, 5));
        assert!(r1.confidence <= 1.0);
        // Full failure
        let r2 = reflector.reflect(&make_execution(false, 0, 5));
        assert!(r2.confidence >= 0.0);
    }
}
