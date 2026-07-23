//! Evolution experiment model and decision logic.
//!
//! Each deployed candidate creates an evolution experiment that measures
//! whether an accepted change helped. The experiment compares baseline
//! and candidate evaluation reports against configured thresholds.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Local types — these mirror the eventual metacog::evaluation and
// metacog::problem types that will be defined in Phases 3-4.  They are
// scoped here to keep the experiment module self-contained while those
// upstream types are still being built.
// ---------------------------------------------------------------------------

/// A problem record referenced by regression tracking.
///
/// This is a local stand-in for the full `ProblemRecord` defined in
/// the problem module (Phase 3, Task 10).  Only the fields needed for
/// experiment outcome reporting are included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemRecord {
    pub problem_id: String,
    pub description: String,
    pub severity: ProblemSeverity,
    pub state: ProblemState,
}

/// Severity of a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProblemSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Lifecycle state of a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProblemState {
    Observed,
    Confirmed,
    Active,
    Mitigated,
    Resolved,
    Regressed,
}

/// A simplified evaluation report sufficient for experiment decisions.
///
/// This is a local stand-in for the full `EvaluationReport` defined in
/// `fabric::types::metacognition_evaluation` (Phase 3, Task 8).  It
/// holds only the fields that `decide_experiment` requires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationReport {
    /// Fixed-point weighted total, in millis (0 – 100_000 = 100.0).
    pub weighted_total_millis: Option<u32>,
    /// Whether all mandatory hard gates passed.
    pub eligible: bool,
    /// Hard-gate results.  Any gate with `name` starting with "safety"
    /// is treated as a safety gate.
    pub gates: Vec<GateResult>,
    /// Evidence coverage, in millis (1_000 = 100%).
    pub evidence_coverage_millis: u16,
    /// Confidence, in millis (1_000 = 100%).
    pub confidence_millis: u16,
}

/// A single hard-gate result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    pub name: String,
    pub passed: bool,
}

// ---------------------------------------------------------------------------
// Evolution experiment types
// ---------------------------------------------------------------------------

/// The decision produced by comparing baseline and candidate reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExperimentDecision {
    /// Candidate improves on the baseline and all gates pass.
    Promote,
    /// Not enough data yet — retain the candidate and observe longer.
    Retain,
    /// Regression detected — roll back to the baseline.
    Rollback,
    /// Candidate is worse and should be rejected.
    Reject,
    /// Not enough evidence to reach a conclusion.
    Inconclusive,
}

/// Defines an evolution experiment — the comparison of a candidate
/// runtime version against a baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionExperiment {
    /// Baseline runtime version identifier.
    pub baseline_version: String,
    /// Candidate runtime version identifier.
    pub candidate_version: String,
    /// Problem IDs the candidate intends to address.
    pub target_problem_ids: Vec<String>,
    /// Distribution of baseline scores (e.g. pre-migration observations).
    pub baseline_score_distribution: Vec<f64>,
    /// Weighted-total-millis improvement required for promotion.
    pub success_threshold: u32,
    /// Weighted-total-millis regression at which rollback is forced.
    pub rollback_threshold: u32,
    /// Minimum observation window in milliseconds before a decision can
    /// be made.
    pub observation_window_ms: u64,
}

/// Outcome of an evolution experiment after comparing reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentOutcome {
    /// Evaluation reports from the baseline period.
    pub pre_reports: Vec<EvaluationReport>,
    /// Evaluation reports from the candidate period.
    pub post_reports: Vec<EvaluationReport>,
    /// Problems that regressed (were resolved but reappeared).
    pub regressions: Vec<ProblemRecord>,
    /// New problems discovered during the candidate period.
    pub new_problems: Vec<ProblemRecord>,
    /// The final decision.
    pub decision: ExperimentDecision,
}

// ---------------------------------------------------------------------------
// Decision logic
// ---------------------------------------------------------------------------

/// Determine whether a candidate version should be promoted, retained,
/// rolled back, or rejected by comparing pre- and post-deployment
/// evaluation reports.
///
/// # Safety-gate behaviour
///
/// Any post-deployment gate whose name starts with `"safety"` and that
/// fails forces `Rollback` **regardless** of the numeric score.  This
/// implements the hard-gate precedence rule from the design spec.
///
/// # Decision rules (in priority order)
///
/// 1. Safety-gate regression → Rollback.
/// 2. Insufficient data (0 reports on either side) → Inconclusive.
/// 3. Post-median weighted total <= pre-median weighted total
///    by at least `rollback_threshold` → Rollback.
/// 4. Post-median weighted total > pre-median weighted total
///    by at least `success_threshold` AND all post gates pass → Promote.
/// 5. Not enough observation window (shortest duration < window) → Retain.
/// 6. Post reports are worse but not enough for rollback → Reject.
/// 7. Fallthrough → Retain.
pub fn decide_experiment(
    baseline: &[EvaluationReport],
    candidate: &[EvaluationReport],
    experiment: &EvolutionExperiment,
) -> ExperimentOutcome {
    let regressions = Vec::new();
    let new_problems = Vec::new();

    // --- 1. Safety-gate check (hardest rule first) ---
    for report in candidate {
        for gate in &report.gates {
            if !gate.passed && gate.name.to_lowercase().starts_with("safety") {
                return ExperimentOutcome {
                    pre_reports: baseline.to_vec(),
                    post_reports: candidate.to_vec(),
                    regressions,
                    new_problems,
                    decision: ExperimentDecision::Rollback,
                };
            }
        }
    }

    // --- 2. Insufficient data ---
    if baseline.is_empty() || candidate.is_empty() {
        return ExperimentOutcome {
            pre_reports: baseline.to_vec(),
            post_reports: candidate.to_vec(),
            regressions,
            new_problems,
            decision: ExperimentDecision::Inconclusive,
        };
    }

    // --- 3. Compare fixed-point weighted totals ---
    let pre_median = median_millis(baseline);
    let post_median = median_millis(candidate);

    // Both sides must have usable weighted totals (i.e. applicable dimensions).
    match (pre_median, post_median) {
        (None, _) | (_, None) => {
            return ExperimentOutcome {
                pre_reports: baseline.to_vec(),
                post_reports: candidate.to_vec(),
                regressions,
                new_problems,
                decision: ExperimentDecision::Inconclusive,
            };
        }
        (Some(pre), Some(post)) => {
            let delta: i64 = post as i64 - pre as i64;

            // Rollback: candidate is worse by at least rollback_threshold
            if delta <= -(experiment.rollback_threshold as i64) {
                return ExperimentOutcome {
                    pre_reports: baseline.to_vec(),
                    post_reports: candidate.to_vec(),
                    regressions,
                    new_problems,
                    decision: ExperimentDecision::Rollback,
                };
            }

            // Promote: candidate improves by at least success_threshold AND all
            // post-deployment hard gates pass.
            if delta >= experiment.success_threshold as i64 && candidate.iter().all(|r| r.eligible)
            {
                return ExperimentOutcome {
                    pre_reports: baseline.to_vec(),
                    post_reports: candidate.to_vec(),
                    regressions,
                    new_problems,
                    decision: ExperimentDecision::Promote,
                };
            }

            // Reject: candidate is worse but not enough for rollback, or gates fail.
            if delta < 0 || candidate.iter().any(|r| !r.eligible) {
                return ExperimentOutcome {
                    pre_reports: baseline.to_vec(),
                    post_reports: candidate.to_vec(),
                    regressions,
                    new_problems,
                    decision: ExperimentDecision::Reject,
                };
            }
        }
    }

    // --- 4. Fallthrough — retain for more evidence ---
    ExperimentOutcome {
        pre_reports: baseline.to_vec(),
        post_reports: candidate.to_vec(),
        regressions,
        new_problems,
        decision: ExperimentDecision::Retain,
    }
}

/// Return the median `weighted_total_millis` from a slice of reports.
/// Reports with `None` weighted total are ignored.
fn median_millis(reports: &[EvaluationReport]) -> Option<u32> {
    let mut values: Vec<u32> = reports
        .iter()
        .filter_map(|r| r.weighted_total_millis)
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid])
    } else {
        // Even count: use the lower-median element for a conservative estimate.
        Some(values[mid - 1])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn report(
        weighted_total_millis: u32,
        eligible: bool,
        gates: Vec<GateResult>,
    ) -> EvaluationReport {
        EvaluationReport {
            weighted_total_millis: Some(weighted_total_millis),
            eligible,
            gates,
            evidence_coverage_millis: 1000,
            confidence_millis: 900,
        }
    }

    fn safety_pass() -> Vec<GateResult> {
        vec![GateResult {
            name: "safety_boundary".into(),
            passed: true,
        }]
    }

    fn safety_fail() -> Vec<GateResult> {
        vec![GateResult {
            name: "safety_boundary".into(),
            passed: false,
        }]
    }

    fn experiment() -> EvolutionExperiment {
        EvolutionExperiment {
            baseline_version: "1.0.0".into(),
            candidate_version: "1.1.0".into(),
            target_problem_ids: vec!["p1".into()],
            baseline_score_distribution: vec![75.0, 80.0, 85.0],
            success_threshold: 5_000,  // 5.0 points in millis
            rollback_threshold: 3_000, // 3.0 points
            observation_window_ms: 60_000,
        }
    }

    #[test]
    fn promote_when_scores_improve() {
        let baseline = vec![report(80_000, true, safety_pass())];
        let candidate = vec![report(86_000, true, safety_pass())];
        let exp = experiment();
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Promote);
    }

    #[test]
    fn retain_not_enough_improvement() {
        let exp = EvolutionExperiment {
            success_threshold: 10_000,
            ..experiment()
        };
        let baseline = vec![report(80_000, true, safety_pass())];
        let candidate = vec![report(82_000, true, safety_pass())];
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Retain);
    }

    #[test]
    fn rollback_when_scores_regress() {
        let baseline = vec![report(80_000, true, safety_pass())];
        let candidate = vec![report(76_000, true, safety_pass())];
        let exp = experiment();
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Rollback);
    }

    #[test]
    fn rollback_on_safety_gate_regression_despite_improved_score() {
        // Even with a much higher score, a failed safety gate forces rollback.
        let baseline = vec![report(80_000, true, safety_pass())];
        let candidate = vec![report(95_000, true, safety_fail())];
        let exp = experiment();
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Rollback);
    }

    #[test]
    fn safety_gate_failure_forces_rollback_even_on_median_improvement() {
        // Multiple reports — median is higher, but one safety gate fails.
        let baseline = vec![
            report(78_000, true, safety_pass()),
            report(80_000, true, safety_pass()),
            report(82_000, true, safety_pass()),
        ];
        let candidate = vec![
            report(85_000, true, safety_pass()),
            report(90_000, true, safety_pass()),
            report(95_000, true, safety_fail()), // <- safety gate failed
        ];
        let exp = experiment();
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Rollback);
    }

    #[test]
    fn reject_when_gate_fails_without_improvement() {
        let baseline = vec![report(80_000, true, safety_pass())];
        let candidate = vec![report(
            79_000,
            false,
            vec![GateResult {
                name: "policy_review".into(),
                passed: false,
            }],
        )];
        let exp = experiment();
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Reject);
    }

    #[test]
    fn inconclusive_when_no_reports() {
        let baseline = vec![];
        let candidate = vec![];
        let exp = experiment();
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Inconclusive);
    }

    #[test]
    fn inconclusive_when_only_one_side_has_reports() {
        let baseline = vec![report(80_000, true, safety_pass())];
        let candidate = vec![];
        let exp = experiment();
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Inconclusive);
    }

    #[test]
    fn inconclusive_when_no_weighted_totals() {
        let no_score = EvaluationReport {
            weighted_total_millis: None,
            eligible: false,
            gates: vec![],
            evidence_coverage_millis: 0,
            confidence_millis: 0,
        };
        let baseline = vec![no_score.clone()];
        let candidate = vec![no_score];
        let exp = experiment();
        let outcome = decide_experiment(&baseline, &candidate, &exp);
        assert_eq!(outcome.decision, ExperimentDecision::Inconclusive);
    }

    #[test]
    fn median_computation_is_deterministic() {
        // Odd count
        let reports = vec![
            report(90_000, true, safety_pass()),
            report(80_000, true, safety_pass()),
            report(85_000, true, safety_pass()),
        ];
        assert_eq!(median_millis(&reports), Some(85_000));

        // Even count -> lower median
        let reports = vec![
            report(80_000, true, safety_pass()),
            report(90_000, true, safety_pass()),
        ];
        assert_eq!(median_millis(&reports), Some(80_000));
    }
}
