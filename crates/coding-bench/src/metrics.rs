use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BenchmarkMetrics {
    pub tasks_run: usize,
    pub first_attempt_success: usize,
    pub repair_iterations: f64,
    pub verifier_pass_rate: f64,
    pub false_success_count: usize,
    pub residual_processes: usize,
    pub crash_resume_success: usize,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub total_elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateThresholds {
    pub first_attempt_success_min: f64,
    pub verifier_pass_rate_min: f64,
    pub false_success_rate_max: f64,
    pub residual_processes_max: usize,
}

impl Default for GateThresholds {
    fn default() -> Self {
        Self {
            first_attempt_success_min: 0.60,
            verifier_pass_rate_min: 0.80,
            false_success_rate_max: 0.05,
            residual_processes_max: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateResult { Pass, Fail(Vec<String>) }

impl BenchmarkMetrics {
    pub fn evaluate(&self, thresholds: &GateThresholds) -> GateResult {
        let mut failures = vec![];
        let total = self.tasks_run as f64;
        if total == 0.0 { return GateResult::Fail(vec!["no tasks run".into()]); }

        let first_rate = self.first_attempt_success as f64 / total;
        if first_rate < thresholds.first_attempt_success_min {
            failures.push(format!("first-attempt {first_rate:.2} < {:.2}",
                thresholds.first_attempt_success_min));
        }
        if self.verifier_pass_rate < thresholds.verifier_pass_rate_min {
            failures.push(format!("verifier {:.2} < {:.2}",
                self.verifier_pass_rate, thresholds.verifier_pass_rate_min));
        }
        let false_rate = self.false_success_count as f64 / total;
        if false_rate > thresholds.false_success_rate_max {
            failures.push(format!("false-success {false_rate:.2} > {:.2}",
                thresholds.false_success_rate_max));
        }
        if self.residual_processes > thresholds.residual_processes_max {
            failures.push(format!("residual processes {}", self.residual_processes));
        }

        if failures.is_empty() { GateResult::Pass } else { GateResult::Fail(failures) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_benchmark_fails_gate() {
        let m = BenchmarkMetrics::default();
        assert_eq!(m.evaluate(&GateThresholds::default()), GateResult::Fail(vec!["no tasks run".into()]));
    }

    #[test]
    fn perfect_scores_pass() {
        let m = BenchmarkMetrics {
            tasks_run: 30,
            first_attempt_success: 24,
            verifier_pass_rate: 0.90,
            false_success_count: 0,
            ..Default::default()
        };
        assert_eq!(m.evaluate(&GateThresholds::default()), GateResult::Pass);
    }

    #[test]
    fn low_success_rate_fails() {
        let m = BenchmarkMetrics {
            tasks_run: 30,
            first_attempt_success: 10,
            verifier_pass_rate: 0.50,
            ..Default::default()
        };
        assert!(matches!(m.evaluate(&GateThresholds::default()), GateResult::Fail(_)));
    }
}
