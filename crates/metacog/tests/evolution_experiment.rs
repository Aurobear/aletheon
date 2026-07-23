//! Tests for evolution experiment decisions — promote, retain, rollback,
//! reject, and inconclusive.  Safety-gate regression must force rollback
//! regardless of score improvement.

use metacog::evolution::experiment::{
    decide_experiment, EvaluationReport, EvolutionExperiment, ExperimentDecision, GateResult,
};

fn report(weighted_total_millis: u32, eligible: bool, gates: Vec<GateResult>) -> EvaluationReport {
    EvaluationReport {
        rubric: fabric::types::metacognition_evaluation::RubricId("experiment-test".into()),
        rubric_version: 1,
        dimensions: Vec::new(),
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
        evidence: Vec::new(),
    }]
}

fn safety_fail() -> Vec<GateResult> {
    vec![GateResult {
        name: "safety_boundary".into(),
        passed: false,
        evidence: Vec::new(),
    }]
}

fn default_experiment() -> EvolutionExperiment {
    EvolutionExperiment {
        baseline_version: "1.0.0".into(),
        candidate_version: "1.1.0".into(),
        target_problem_ids: vec!["p1".into()],
        baseline_score_distribution: vec![75.0, 80.0, 85.0],
        success_threshold: 5_000,
        rollback_threshold: 3_000,
        observation_window_ms: 60_000,
        observed_duration_ms: 60_000,
    }
}

// ---------------------------------------------------------------------------
// Promote
// ---------------------------------------------------------------------------

#[test]
fn promote_when_scores_improve_and_gates_pass() {
    let baseline = vec![report(80_000, true, safety_pass())];
    let candidate = vec![report(86_000, true, safety_pass())];
    let outcome = decide_experiment(&baseline, &candidate, &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Promote);
}

#[test]
fn promote_with_multiple_reports_median_improvement() {
    let baseline = vec![
        report(78_000, true, safety_pass()),
        report(80_000, true, safety_pass()),
        report(82_000, true, safety_pass()),
    ];
    let candidate = vec![
        report(84_000, true, safety_pass()),
        report(86_000, true, safety_pass()),
        report(88_000, true, safety_pass()),
    ];
    // median: 80_000 -> 86_000, delta 6_000 > success_threshold 5_000
    let outcome = decide_experiment(&baseline, &candidate, &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Promote);
}

// ---------------------------------------------------------------------------
// Retain — not enough evidence to decide yet
// ---------------------------------------------------------------------------

#[test]
fn retain_when_improvement_is_below_success_threshold() {
    let exp = EvolutionExperiment {
        success_threshold: 10_000,
        ..default_experiment()
    };
    let baseline = vec![report(80_000, true, safety_pass())];
    let candidate = vec![report(82_000, true, safety_pass())];
    let outcome = decide_experiment(&baseline, &candidate, &exp);
    assert_eq!(outcome.decision, ExperimentDecision::Retain);
}

#[test]
fn retain_until_observation_window_completes() {
    let exp = EvolutionExperiment {
        observed_duration_ms: 59_999,
        ..default_experiment()
    };
    let baseline = vec![report(80_000, true, safety_pass())];
    let candidate = vec![report(90_000, true, safety_pass())];
    assert_eq!(
        decide_experiment(&baseline, &candidate, &exp).decision,
        ExperimentDecision::Retain
    );
}

// ---------------------------------------------------------------------------
// Rollback
// ---------------------------------------------------------------------------

#[test]
fn rollback_when_candidate_median_drops_below_rollback_threshold() {
    let baseline = vec![report(80_000, true, safety_pass())];
    let candidate = vec![report(76_000, true, safety_pass())];
    let outcome = decide_experiment(&baseline, &candidate, &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Rollback);
}

#[test]
fn rollback_on_safety_gate_regression_despite_score_improvement() {
    // Score jumps from 80_000 -> 95_000 but safety gate fails.
    let baseline = vec![report(80_000, true, safety_pass())];
    let candidate = vec![report(95_000, true, safety_fail())];
    let outcome = decide_experiment(&baseline, &candidate, &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Rollback);
}

#[test]
fn rollback_when_only_one_of_many_candidate_reports_fails_safety() {
    let baseline = vec![
        report(78_000, true, safety_pass()),
        report(80_000, true, safety_pass()),
        report(82_000, true, safety_pass()),
    ];
    let candidate = vec![
        report(85_000, true, safety_pass()),
        report(90_000, true, safety_pass()),
        report(95_000, true, safety_fail()),
    ];
    let outcome = decide_experiment(&baseline, &candidate, &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Rollback);
}

// ---------------------------------------------------------------------------
// Reject
// ---------------------------------------------------------------------------

#[test]
fn reject_when_candidate_worse_and_not_enough_for_rollback() {
    let baseline = vec![report(80_000, true, safety_pass())];
    let candidate = vec![report(79_500, true, safety_pass())];
    // delta = -500, rollback_threshold is 3_000 so not enough for rollback
    let outcome = decide_experiment(&baseline, &candidate, &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Reject);
}

#[test]
fn reject_when_candidate_has_failed_non_safety_gate() {
    let baseline = vec![report(80_000, true, safety_pass())];
    let candidate = vec![report(
        80_000,
        false,
        vec![GateResult {
            name: "policy_review".into(),
            passed: false,
            evidence: Vec::new(),
        }],
    )];
    let outcome = decide_experiment(&baseline, &candidate, &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Reject);
}

// ---------------------------------------------------------------------------
// Inconclusive
// ---------------------------------------------------------------------------

#[test]
fn inconclusive_when_no_reports_on_baseline() {
    let candidate = vec![report(86_000, true, safety_pass())];
    let outcome = decide_experiment(&[], &candidate, &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Inconclusive);
}

#[test]
fn inconclusive_when_no_reports_on_candidate() {
    let baseline = vec![report(80_000, true, safety_pass())];
    let outcome = decide_experiment(&baseline, &[], &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Inconclusive);
}

#[test]
fn inconclusive_when_no_reports_at_all() {
    let outcome = decide_experiment(&[], &[], &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Inconclusive);
}

#[test]
fn inconclusive_when_weighted_totals_are_none() {
    let no_score = EvaluationReport {
        rubric: fabric::types::metacognition_evaluation::RubricId("experiment-test".into()),
        rubric_version: 1,
        dimensions: Vec::new(),
        weighted_total_millis: None,
        eligible: false,
        gates: vec![],
        evidence_coverage_millis: 0,
        confidence_millis: 0,
    };
    let outcome = decide_experiment(&[no_score.clone()], &[no_score], &default_experiment());
    assert_eq!(outcome.decision, ExperimentDecision::Inconclusive);
}
