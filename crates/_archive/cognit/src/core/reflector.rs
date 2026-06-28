//! Reflector — post-execution reflection.
//!
//! Analyzes execution results to determine what worked, what failed,
//! and what should be improved. Produces Reflection structs that feed
//! into the Learner for rule extraction.
//!
//! Also produces structured ReflectionEntry for persistent self-evolution.

use base::brain::{
    ExecutionResult, Reflection, ReflectionEntry, ReflectionOutcome, ReflectionTrigger,
};
use chrono::Utc;
use uuid::Uuid;

/// The reflector component.
///
/// Performs post-execution analysis to extract lessons from outcomes.
#[derive(Clone)]
pub struct Reflector;

impl Reflector {
    pub fn new() -> Self {
        Self
    }

    /// Reflect on an execution result.
    ///
    /// Extracts specific information from `output` (successes) and `error` (failures)
    /// rather than emitting generic templates.
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

            // Extract specific success signals from output
            for line in execution.output.lines() {
                let lower = line.to_lowercase();
                if lower.contains("created") || lower.contains("deployed") || lower.contains("installed") {
                    what_worked.push(format!("Output indicates: {}", line.trim()));
                }
                if lower.contains("warning") || lower.contains("degraded") || lower.contains("partial") {
                    what_to_improve.push(format!("Output warning: {}", line.trim()));
                }
            }
        } else {
            what_failed.push(format!(
                "Plan {} failed at step {}/{}.",
                execution.plan_id, execution.steps_completed, execution.steps_total
            ));

            // Extract specific failure details from error
            if let Some(ref error) = execution.error {
                let error_lines: Vec<&str> = error.lines().collect();
                if error_lines.len() > 1 {
                    // Multi-line error: capture first line as summary, last non-empty as root cause
                    what_failed.push(format!("Error summary: {}", error_lines[0].trim()));
                    if let Some(root) = error_lines.iter().rev().find(|l| !l.trim().is_empty()) {
                        what_failed.push(format!("Root cause: {}", root.trim()));
                    }
                } else {
                    what_failed.push(format!("Error: {}", error));
                }

                // Suggest specific fixes based on error content
                let lower = error.to_lowercase();
                if lower.contains("permission") || lower.contains("access denied") {
                    what_to_improve
                        .push("Check file/resource permissions before retrying.".to_string());
                } else if lower.contains("not found") || lower.contains("no such") {
                    what_to_improve
                        .push("Verify resource exists and path/identifier is correct.".to_string());
                } else if lower.contains("timeout") || lower.contains("timed out") {
                    what_to_improve
                        .push("Increase timeout or check network/service availability.".to_string());
                } else if lower.contains("connection") || lower.contains("refused") {
                    what_to_improve
                        .push("Verify service is running and network connectivity is available.".to_string());
                } else if lower.contains("parse") || lower.contains("syntax") || lower.contains("invalid") {
                    what_to_improve
                        .push("Validate input format and fix syntax errors before retrying.".to_string());
                } else {
                    what_to_improve
                        .push("Add error handling and retry logic for this failure mode.".to_string());
                }
            } else {
                what_to_improve
                    .push("Add error handling and retry logic for failed steps.".to_string());
            }

            what_to_improve
                .push("Consider adding rollback actions for partial completion.".to_string());
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
            what_to_improve
                .push("Execution produced no output — verify expected outcomes.".to_string());
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

    /// Produce a structured ReflectionEntry for persistent storage.
    pub fn reflect_entry(
        &self,
        task_summary: &str,
        trigger: ReflectionTrigger,
        execution: &ExecutionResult,
    ) -> ReflectionEntry {
        let reflection = self.reflect(execution);

        let outcome = if execution.success {
            if execution.steps_completed < execution.steps_total {
                ReflectionOutcome::Partial
            } else {
                ReflectionOutcome::Success
            }
        } else {
            ReflectionOutcome::Failure
        };

        // Derive learned lessons from what_to_improve
        let learned: Vec<String> = reflection
            .what_to_improve
            .iter()
            .map(|s| s.clone())
            .collect();

        ReflectionEntry {
            id: format!("reflect-{}", Uuid::new_v4()),
            timestamp: Utc::now(),
            trigger,
            task_summary: task_summary.to_string(),
            outcome,
            what_worked: reflection.what_worked,
            what_failed: reflection.what_failed,
            learned,
            behavior_changes: vec![], // Populated by ExperienceSummarizer in Phase 2
            confidence: reflection.confidence,
        }
    }

    /// Produce a ReflectionEntry from a conversational task (no ExecutionResult).
    ///
    /// Used for chat-based tasks where we reflect on the conversation itself.
    pub fn reflect_conversation(
        &self,
        task_summary: &str,
        trigger: ReflectionTrigger,
        success: bool,
        what_worked: Vec<String>,
        what_failed: Vec<String>,
        learned: Vec<String>,
    ) -> ReflectionEntry {
        let confidence = if success { 0.8 } else { 0.3 };

        ReflectionEntry {
            id: format!("reflect-{}", Uuid::new_v4()),
            timestamp: Utc::now(),
            trigger,
            task_summary: task_summary.to_string(),
            outcome: if success {
                ReflectionOutcome::Success
            } else {
                ReflectionOutcome::Failure
            },
            what_worked,
            what_failed,
            learned,
            behavior_changes: vec![],
            confidence,
        }
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
            error: if success {
                None
            } else {
                Some("test error".to_string())
            },
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
        assert!(result
            .what_to_improve
            .iter()
            .any(|i| i.contains("optimizing")));
    }

    #[test]
    fn empty_output_noted() {
        let reflector = Reflector::new();
        let mut exec = make_execution(true, 1, 1);
        exec.output = String::new();
        let result = reflector.reflect(&exec);
        assert!(result
            .what_to_improve
            .iter()
            .any(|i| i.contains("no output")));
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

    // --- reflect_entry() tests ---

    #[test]
    fn reflect_entry_success_outcome() {
        let reflector = Reflector::new();
        let entry = reflector.reflect_entry(
            "deploy service",
            ReflectionTrigger::TaskComplete,
            &make_execution(true, 3, 3),
        );
        assert_eq!(entry.outcome, ReflectionOutcome::Success);
        assert_eq!(entry.trigger, ReflectionTrigger::TaskComplete);
        assert_eq!(entry.task_summary, "deploy service");
        assert!(entry.id.starts_with("reflect-"));
        assert!(!entry.what_worked.is_empty());
        assert!(entry.what_failed.is_empty());
        assert!(entry.confidence > 0.8);
    }

    #[test]
    fn reflect_entry_failure_outcome() {
        let reflector = Reflector::new();
        let entry = reflector.reflect_entry(
            "run tests",
            ReflectionTrigger::TaskComplete,
            &make_execution(false, 1, 3),
        );
        assert_eq!(entry.outcome, ReflectionOutcome::Failure);
        assert!(!entry.what_failed.is_empty());
        assert!(!entry.learned.is_empty());
        assert!(entry.confidence < 0.5);
    }

    #[test]
    fn reflect_entry_partial_outcome() {
        let reflector = Reflector::new();
        let entry = reflector.reflect_entry(
            "partial task",
            ReflectionTrigger::Impasse,
            &make_execution(true, 2, 5),
        );
        assert_eq!(entry.outcome, ReflectionOutcome::Partial);
        assert_eq!(entry.trigger, ReflectionTrigger::Impasse);
        assert!(entry.learned.iter().any(|l| l.contains("partial")));
    }

    #[test]
    fn reflect_entry_behavior_changes_empty() {
        let reflector = Reflector::new();
        let entry = reflector.reflect_entry(
            "any task",
            ReflectionTrigger::Manual,
            &make_execution(true, 1, 1),
        );
        assert!(entry.behavior_changes.is_empty());
    }

    // --- reflect_conversation() tests ---

    #[test]
    fn reflect_conversation_success() {
        let reflector = Reflector::new();
        let entry = reflector.reflect_conversation(
            "explain architecture",
            ReflectionTrigger::Manual,
            true,
            vec!["clear explanation".to_string()],
            vec![],
            vec!["user prefers diagrams".to_string()],
        );
        assert_eq!(entry.outcome, ReflectionOutcome::Success);
        assert_eq!(entry.trigger, ReflectionTrigger::Manual);
        assert_eq!(entry.task_summary, "explain architecture");
        assert_eq!(entry.what_worked, vec!["clear explanation"]);
        assert!(entry.what_failed.is_empty());
        assert_eq!(entry.learned, vec!["user prefers diagrams"]);
        assert!(entry.confidence > 0.7);
    }

    #[test]
    fn reflect_conversation_failure() {
        let reflector = Reflector::new();
        let entry = reflector.reflect_conversation(
            "debug crash",
            ReflectionTrigger::Impasse,
            false,
            vec!["identified stack trace".to_string()],
            vec!["could not reproduce".to_string()],
            vec!["need more logs".to_string()],
        );
        assert_eq!(entry.outcome, ReflectionOutcome::Failure);
        assert_eq!(entry.trigger, ReflectionTrigger::Impasse);
        assert_eq!(entry.what_failed, vec!["could not reproduce"]);
        assert_eq!(entry.learned, vec!["need more logs"]);
        assert!(entry.confidence < 0.5);
    }

    #[test]
    fn reflect_conversation_id_unique() {
        let reflector = Reflector::new();
        let e1 = reflector.reflect_conversation(
            "task a",
            ReflectionTrigger::Manual,
            true,
            vec![],
            vec![],
            vec![],
        );
        let e2 = reflector.reflect_conversation(
            "task b",
            ReflectionTrigger::Manual,
            false,
            vec![],
            vec![],
            vec![],
        );
        assert_ne!(e1.id, e2.id);
    }

    #[test]
    fn reflect_conversation_empty_lists() {
        let reflector = Reflector::new();
        let entry = reflector.reflect_conversation(
            "noop",
            ReflectionTrigger::TaskComplete,
            true,
            vec![],
            vec![],
            vec![],
        );
        assert!(entry.what_worked.is_empty());
        assert!(entry.what_failed.is_empty());
        assert!(entry.learned.is_empty());
        assert!(entry.behavior_changes.is_empty());
    }
}
