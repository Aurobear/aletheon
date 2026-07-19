//! Benchmark runner — executes tasks and aggregates metrics.

use crate::metrics::{BenchmarkMetrics, GateResult, GateThresholds};
use crate::tasks::{BenchmarkTask, TaskCategory};

pub struct RunResult {
    pub task_id: usize,
    pub category: TaskCategory,
    pub first_attempt_ok: bool,
    pub repair_attempts: usize,
    pub verifier_passed: bool,
    pub residual_processes: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub elapsed_ms: u64,
}

pub struct BenchmarkRunner {
    tasks: Vec<BenchmarkTask>,
    thresholds: GateThresholds,
}

impl BenchmarkRunner {
    pub fn new(tasks: Vec<BenchmarkTask>) -> Self {
        Self { tasks, thresholds: GateThresholds::default() }
    }

    pub fn with_thresholds(mut self, t: GateThresholds) -> Self { self.thresholds = t; self }

    /// Run all tasks through a synchronous executor callback.
    pub fn run(&self, mut executor: impl FnMut(&BenchmarkTask, usize) -> RunResult) -> Vec<RunResult> {
        self.tasks.iter().enumerate().map(|(i, t)| {
            let mut result = executor(t, i);
            result.task_id = i;
            result
        }).collect()
    }

    pub fn aggregate(&self, results: &[RunResult]) -> BenchmarkMetrics {
        let n = results.len();
        BenchmarkMetrics {
            tasks_run: n,
            first_attempt_success: results.iter().filter(|r| r.first_attempt_ok).count(),
            repair_iterations: if n > 0 { results.iter().map(|r| r.repair_attempts).sum::<usize>() as f64 / n as f64 } else { 0.0 },
            verifier_pass_rate: if n > 0 { results.iter().filter(|r| r.verifier_passed).count() as f64 / n as f64 } else { 0.0 },
            false_success_count: results.iter().filter(|r| r.verifier_passed && !r.first_attempt_ok).count(),
            residual_processes: results.iter().map(|r| r.residual_processes).sum(),
            crash_resume_success: 0,
            total_tokens_in: results.iter().map(|r| r.tokens_in).sum(),
            total_tokens_out: results.iter().map(|r| r.tokens_out).sum(),
            total_elapsed_ms: results.iter().map(|r| r.elapsed_ms).sum(),
        }
    }

    pub fn evaluate(&self, executor: impl FnMut(&BenchmarkTask, usize) -> RunResult) -> GateResult {
        let results = self.run(executor);
        self.aggregate(&results).evaluate(&self.thresholds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tasks() -> Vec<BenchmarkTask> {
        (0..30).map(|i| BenchmarkTask { category: TaskCategory::SearchExplain, description: format!("t{i}"), repo_path: ".".into(), acceptance: vec![] }).collect()
    }

    #[test]
    fn runner_aggregates_30_tasks() {
        let r = BenchmarkRunner::new(tasks());
        let results = r.run(|_, _| RunResult { task_id: 0, category: TaskCategory::SearchExplain, first_attempt_ok: true, repair_attempts: 0, verifier_passed: true, residual_processes: 0, tokens_in: 100, tokens_out: 50, elapsed_ms: 100 });
        assert_eq!(results.len(), 30);
        let m = r.aggregate(&results);
        assert_eq!(m.tasks_run, 30);
        assert_eq!(m.first_attempt_success, 30);
    }

    #[test]
    fn all_passing_passes_gate() {
        let r = BenchmarkRunner::new(tasks());
        assert_eq!(r.evaluate(|_, _| RunResult { task_id:0, category: TaskCategory::SearchExplain, first_attempt_ok:true, repair_attempts:0, verifier_passed:true, residual_processes:0, tokens_in:0, tokens_out:0, elapsed_ms:0 }), GateResult::Pass);
    }

    #[test]
    fn all_failing_fails_gate() {
        let r = BenchmarkRunner::new(tasks());
        assert!(matches!(r.evaluate(|_, _| RunResult { task_id:0, category: TaskCategory::SearchExplain, first_attempt_ok:false, repair_attempts:5, verifier_passed:false, residual_processes:1, tokens_in:0, tokens_out:0, elapsed_ms:0 }), GateResult::Fail(_)));
    }
}
